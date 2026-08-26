# Gates · NOW.md — what next

The only list that answers "what should the loop pick up." Done items are
deleted, not checked — history lives in git and `DECISIONS.md`. An item is
≤ ~25 lines (`CLAUDE.md` §loop discipline); detail belongs in
`DECISIONS.md` §open or a `findings/` note.

> **Rebuilt 2026-08-25: 3,273 → 1,718 lines, 82 items → 76.** Every
> section was re-read against the tree with commands rather than against its
> own memory, and what came back is that almost nothing here was *finished* —
> it was **buried**. Two sections were closed outright (`0kit`, `5c`); the
> other eighty were nine parts landed narrative to one part live work, so what
> is deleted is the story of what already shipped and what is kept is the
> remainder, with a `file:line` on it.
>
> **Nothing open was dropped, and that was checked rather than claimed.** The
> rewrite was diffed against the original a second time, adversarially, asking
> only *what live work went missing* — and it found **nineteen** items, each
> re-verified against the tree before being restored: the head-look spine
> follow-up, the tech-tree panel's edges, the stairs-drawn-as-a-plate, the
> fleet raiding itself, what the `prove` slice actually costs, and fourteen
> more. One thing it flagged was **correctly** dropped and stays dropped: the
> `bevy_procedural_tree` bug report, which `render/tree.rs:241` says was filed.
> A prune that is not diffed back is a prune that loses things quietly.
>
> **Three sections that read as settled are not, and each was found by a
> command.** `5b` says **CLOSED** and covers two of four over-wide wire
> domains — the craft-refused and deploy-refused reasons are raw octets,
> unchecked at both ends and absent from `event.rs`'s `DOMAINS` table. `0zd`'s
> *"not owed, do not re-litigate"* rests on a blocker that **died**:
> `ItemStack` gained `cond: u16` in durability v0, which is the per-item
> instance data the key lock was refused for. `0bd`'s tree row is half-closed:
> `OCCUPANT_TOP_M[Tree]` cites a dead-code far-LOD constant, so the sim blocks
> 0.3 m of invisible ceiling over half the pool.
>
> **The queue's real shape: 25 of these items are the same one act** — not
> code, but a person booting the game on a machine with a GPU and looking. `§LOOK` at the
> bottom is that list, in one place for the first time; the swing, the decal,
> the death pose, LOW and MEDIUM, the far forest, the broadleaf, the announce
> stack and the whole audio bank have never been seen or heard by anyone.
> `CLAUDE.md` says the visual gate is a person and forbids building a pixel
> gate to replace them, so this is the bottleneck by design — but it had never
> been counted, and it is the largest single thing standing between this tree
> and a playtest.
>
> ⚠ **Section labels still collide** (`0a 0u 0v 0w 0x 0y 0z 4b`) because
> `merge=union` lets each lane pick "the next free letter" against a file that
> does not hold the others' picks. They are **not** renumbered — the citations
> are mostly in `DECISIONS.md`, which is the dated record and is not rewritten
> to match a later tidy — so read a `§`-citation as a hint and match on the
> title. `§Labels` at the end says which are ambiguous and what was deleted.
> When you next edit a colliding section, give it a label no other section has.
>
> ⚠ **One section had lost its heading entirely** and had been eaten by `0bl`
> for long enough that two crate comments cite it by a label the file did not
> contain. It is `§0sun` now.

---

# Buildable now — a loop can pick any of these


## Sim, content and gameplay verbs *(systems lane)*


## 5 · Gameplay still missing, in rough order of what a player notices

Items 1–3 are a **spoken operator call**, not a builder's proposal — 2026-08-10:
*"ranged tracks the reference game as closely as we can, and arrows come back"*
(`DECISIONS.md`; `reference/PROJECTILES.md` §9 is the sized list).

1. **The arrow does no structure damage.** An arrow that reaches a wall
   stops dead rather than chipping it (`sim-core/src/ranged.rs:50`). ⚠ The
   fat-arrow half of this bullet is done — `collide::shot_blocked` takes a
   radius and pieces stop an arrow at `ARROW_R_M`.
2. **Arrow recovery** (`reference/PROJECTILES.md` §9.7) — the spent-arrow
   store, the ~15 % break, the 10 s lodge, and the first verb in the
   protocol addressed to a world position rather than a build cell. A
   protocol pass, not an afternoon. It gates §9.6: no bow number may track
   theirs until arrows come back.
3. **`headshot_mult` is armed and unread** since the content crate (§9.4);
   §7 says take the most significant body part, never the first
   intersection.
4. **A forest-floor pickup archetype, and a farming lane.** Both code.
   `server/tests/farmwalk.rs` measures a gather rate; it is not farming.
5. **The tech tree is one edge deep.** `requires` and the `validate`
   reachability check ship, and `bake_research` has a caller now, but every
   row in `content/research.toml` depends on a root. Still absent: a
   blueprint ITEM (learning is instant and personal, so there is nothing to
   trade), and the wipe schedule `DESIGN.md` §8 promises blueprints will
   outlive.
6. **Day/night reads nothing but mobs.** ⚠ `mob::think` is nocturnal now
   (`world::is_night`). Still missing: no crops, no torch, no moon or stars
   in the night sky, and no set-time verb — moving the clock means moving
   the tick.


## 0pvp · What a fight still cannot do *(systems lane)*

1. **The flinch is attacker-side only** — `EV_HIT` is unicast, so the recoil is
   on one screen; the symmetric version is a new broadcast, unpriced
   (`DECISIONS.md` §open "attacker-side flinch v0"). Nobody has seen the pose.
2. **No positional hit sound** — a flesh impact needs a waveform `sound/
   synth.rs` does not generate. Nobody has heard `Cue::RemoteSwing` either.
3. **A gun has no muzzle flash, crack or tracer**: `ranged::hitscan` emits no
   `EV_SHOT`, so a firearm speaks only through `EV_IMPACT`/`EV_HIT`. A voice is
   a new event or a spoken reading of `EV_SHOT`'s spare patterns. ⚠ A firearm
   death still reports `DEATH_BY_ARROW`.
4. **Nothing can EQUIP armor — an operator call, three exits**: the wire
   (`CONT_WEAR`: `CONT_KIND_BITS` is still 2 and all four values are spent;
   `PROTO_VER` is **50** now, not the 48 → 49 this was written against;
   `findings/armor-design-20260818.md` §4 prices it), a spoken spawn-wear
   default, or auto-protect. ⚠
   `balance.rs`'s anchor is known-misleading and left so — needs
   `armor_extra_hits_max` re-spoken or the ladder re-priced. Open too: damage
   types, hit areas, condition, `move_penalty_pct`.
5. **No lag compensation** — slices 2–5 of `findings/lagcomp-design-20260818.md`
   §7: the ring in `sim-core`, `Command::Input` carrying `favour`, `strike`
   rewinding, the server minting it. None exists yet; no wire bump is owed.
6. **Nothing has fought at population** (`raid_storm.rs:516`: *"nobody
   swings"*). Plan: `findings/combat-soak-design-20260818.md`; the cheapest
   slice is no code — `sim-core/src/bots.rs:53-60` presses `BTN_PRIMARY` 1-in-3.


## 0mk · A swing at a piece marks nothing, and arrows pass through floors *(systems+client lane)*

1. **A swing at a built PIECE marks nothing.** `combat::raid`
   (`combat.rs:869`) pushes no `EV_IMPACT`; `SURF_BUILT` is the kind, and
   it sits behind item 2. Flesh stays unmarked by choice (one spare code).
2. **⚠ Fix the collision, not the decal.** `collide::shot_blocked`
   (`collide.rs:1266`) calls only
   `cell_edges_stop_shot` and `cell_diags_block`, never `ColIndex::planes`
   (`collide.rs:126`) — so an arrow fired down inside a base passes through
   every floor as `SURF_GROUND`. `plane_blocked`'s doc sends this to
   `NOW.md` §0ar, **a section that does not exist**. Only then the piece
   address on the message: 27 bits against 4 spare pad bits, 11 bytes.
   **What IS reachable today is a diagonal wall, 45° out** — `decal.rs`'s
   built-piece arm snaps the normal to the dominant horizontal axis (±X or
   ±Z), so the one wrong mark a player can actually see is on a diagonal.
3. **Spray paint is a deployable, not a decal**: a `limits.rs` cap, a
   `worldsave.rs` slot, build privilege, decay, moderation. Stencil or
   painted is the call to make first.

⚠ **Nobody has seen a decal**: no `ForwardDecal` renders under lavapipe at
any size, alpha or orientation. One boot on a real GPU settles it.


## 0wc · What world containers v0 still owes *(systems lane)*

1. **Nobody has opened one in the running game** — the prompt, the panel
   title, the drag out of a 30-slot grid, an emptied crate. Route: derive
   the anchor as `container_wire.rs:1307` does, set `dev_spawn` in
   `shard.toml` (`server/src/config.rs:361`), boot. §0p3 has the command.
2. **An emptied crate says nothing at a distance**, so a wasted trip is
   normal on a populated shard. Wants a lid state on the mesh
   (`render/props.rs` has one `crate_box`) or a shorter refill window.
3. **The guard has no loot tier of its own** — `guard.rs`'s
   `a_guard_pays_what_a_wolf_pays` holds it to a wolf's meat and fat. A
   tier wants a third species, and a third kind still falls through to
   the pig in `render/mobs.rs`, `sound/voice.rs` and `ui/death.rs`;
   `loot.toml` cannot carry it (`content/src/validate.rs:887` refuses
   zero hits).
4. **Nobody has fought a guard in the running game.** Same as 1.
5. **`inventory.rs:110`'s `slots_in` is the same defect one function
   over** — `CONT_BOX => BOX_SLOTS, _ => INV_SLOTS`, right only because a
   world container is `INV_SLOTS` wide. ⚠ The gate that looks like coverage
   is not: `a_world_crate_is_drawn_from_the_crate_store` reads `0..INV_SLOTS`,
   so a fifth ground kind of a different width draws the wrong slot count
   silently and that test stays green. Wants an explicit arm under
   `container_wire.rs:1359`'s `CONT_MAX` compile guard.


## 0pr · What predator v0 still owes *(systems lane)*

1. **Nobody has heard any of it.** `client/src/bin/soundbank.rs` dumps the
   bank to WAV; ears are the gate that has not run. Listen for the two
   cadences (`HOWL_PERIOD_S` 75 s, `GROWL_PERIOD_S` 2.5 s), the 0.5×
   night sense and the 16 predators — all four are arithmetic.
2. **A wolf pays no hide and no bone** — `content/mobs.toml` drops meat
   and fat only; refused in the roster slice because it drags recipes and
   `ui::icons::STEMS` in with it.
3. **Night still costs the player nothing.** Nocturnal senses made the
   hour a tactic, not the dark dangerous. The sourced follow-on is **not**
   more tuning of `night_spook_cm` — it is a night-only roster variant
   (Minecraft and Valheim gate *spawns* on darkness). The judge's gap 1
   wanted a warmth stat; `survival.rs:60` still records no temperature.
4. **The growl radius has no gate.** `sound/mod.rs:565` names §0pr as
   holding it: `CUES[Growl].radius_m` (14 m) must stay inside the wolf's
   night notice radius (15 m), and a `mobs.toml` edit reddens nothing.


## 0m · The pig is in — what the roster still owes *(systems lane)*

Research `reference/ANIMALS.md` §9.5. A wolf joined the roster since
this was written (predator v0), so it is no longer just the pig.

1. **A butchering VERB** — a tool-gated harvest on the body.
   `ui::interact::Verb` has no arm for it; the corpse bag
   (`mob::strike` → `backpack::stand_up`) is where its output goes.
2. **The combat-feel half of mob attack is minimal** — the victim sees
   hp drop and hears nothing species-specific (`sound/voice.rs` is
   presence, not reaction), so an aggro cue and a damage-direction tick
   are owed, and the charge costs the mob nothing to hold.
3. **The massing is boxy up close** — at 8 m the head barely separates
   from the body; `render/mobs.rs` is still a box massing.
4. **`MAX_MOBS = 64` has never met a playtest** — derived from the wire
   budget rather than felt, and the one number a player answers.
5. **Whether `ttk_melee` should widen** so a rock is worse than a
   crafted spear by more than one hit — `DECISIONS.md` §open, "tools as
   weapons".


## 0ctl · Four controls the player expects and the sim has no verb for *(systems lane)*

Bind each **in the commit that gives it a verb**: all four re-confirmed
unbuilt, and a key that does nothing is worse than an absent one.

1. **Reload (`R`).** No magazine, loaded state or reload verb anywhere;
   `ranged::draw`/`hitscan` spend from the inventory. Needs loaded-round
   state on the stack — `0dur`'s `ItemStack` question; `R` is repair.
2. **ADS / secondary (RMB).** No `BTN_SECONDARY`; `BTN_MASK` holds four
   bits, and RMB is already deploy-place, the build wheel and the half-stack
   grab. Needs a held-item modality answer before a bit (`PROTO_VER` bump).
3. **Flashlight (`F`).** No held light source; `item.torch` is inert and
   `tests/held_assets.rs::nothing_held_glows` forbids a carried emissive by
   name — so this starts by deciding that test's fate.
⚠ **Both keys are already conditionally bound**: `ghost.rs:153/156` give `R`
and `F` the build ghost's level up/down while the wheel is up, and
`verbs.rs:245` is `R`'s repair arm otherwise. Bind over them knowingly.
4. **Voice (hold `V`).** No capture, codec, `KIND_*` or fan-out;
   `reference/VOICE.md` §9 settles both design questions.
Also open: the viewmodel sways in free look (`viewmodel.rs` reads `eye.yaw`
= `look.yaw + look.free_yaw`). §open "free look v0".


## 0sp2 · What the spill still cannot say *(systems lane)*

The first two need a wire field (`DECISIONS.md` §open):

- **A partial spill is still invisible.** Some fits, some does not, and
  the shortfall never leaves the sim — the wire carries what reached the
  hands and never what was paid, so `+3 × Wood` cannot say the other 7
  fell. The ring is `client-core/src/core.rs:900`, item index only.
- **The four give-backs say nothing at all** — demolish refund, pick-up,
  unbolt, craft cancel emit no payout event, spilled or not. Operator:
  those two together are what a wire field buys.
- **The merge ignores ownership** — a spill lands in whatever bag is
  nearest, including someone else's death bag
  (`sim-core/src/backpack.rs:51`). §open carries it.
- **Nobody has seen one.** Proven headless only, the "pack full — Wood
  dropped at your feet" line included: no frame in this repo has ever
  shown it.


## 0bl · The lattice's residuals: a seam, a memo, and a shot with no flanks *(client+sim lane)*

1. **A band-boundary wall bases on its canonical cell** and hangs one band over
   the lower plate — an arrow-sized slit. The lower column is the honest base;
   needs `collide` and the renderer together. Rare since the plate, not fixed.
2. **The flank costs 153 µs a tick; a memo takes most of it back.** `col_base_y`
   re-samples terrain per cell per candidate and `plane_blocked` reads four
   cells. `build::terrain_band` is pure in (seed, cell), so a direct-mapped memo
   is exact (`occupy::SlotCache`'s argument). Nothing memoizes it. Not urgent.
3. **The shot walk ignores the flanks** — `collide::shot_blocked` reads edges
   and diagonals only, so an arrow passes through a floor a body is stopped by.
   ⚠ this pointed at a §0ar that does not exist in this file.
4. **The half wall** — the reference's answer to the gap a half-storey plate
   offset leaves on upper floors; `build.rs` has eleven `SHAPE_*`, no half.
5. **The stepped foundation — and DO NOT widen the plate limits instead.**
   `reference/BUILDING.md` §7c.2 is a published, tested negative result on
   exactly that change: they tried a three-metre gradient on `foundation.steps`
   for our problem, it helped mountains, hurt flats and clipped their door
   blocks, and they reverted it. Ours is a catalogue row plus a shape code
   (§9 item 18), never a knob. Recorded here because it will keep suggesting
   itself.
6. **The diagonal wall's √2 root scale stretches its UVs** —
   `render/structures.rs:1272` turns the slab ±45° and scales `SQRT_2` along
   its length. Pinned so it cannot grow; `ART.md`'s business, not a defect.
7. **Operator:** whether `place` should refuse a piece whose cell a body stands
   in (`DECISIONS.md` §open "piece flanks v0"), and nobody has played the aimed
   freehand bit — which rides no golden either, closing which means scripting
   `sim-core/src/probe.rs` to build beside a built neighbour.


## 0ac · The catalogue's inserts, the soft face's look, and the diagonal price *(systems lane)*

1. **The inserts are unbuilt** — bars, glass, shutters, the garage door
   (`reference/BUILDING.md` §7b.4's second purchase, §9.13's remainder).
   Each is a deployable pass of its own; `content/building.toml` says so
   at both socket rows, and `place_deploy` still requires
   `SHAPE_DOORWAY`.
2. **The soft face has no visual identity.** `build::soft_side` prices
   the swing and labels the HUD prompt; nothing in
   `render/structures.rs` reads it, so the label is the only tell. Also
   owed: floor sides (needs a vertical attack direction) and the pairing
   with `RIPLIST.md` §2's per-material resistance.
3. **Triangles want a look and a price call**: a capture pass on a
   diagonal base in the booted game (the person is the visual gate); the
   wall-on-diagonal price — ~1.41× the length, today priced by the
   socket (`DECISIONS.md` §open "triangles v0", open for the operator,
   with the wheel at 11 wedges); and hard/soft's identity on tri halves.


## 0tt · The bench ladder's craft rebate, unbuilt *(systems lane)*

1. **The craft rebate** (`RIPLIST.md` §2 row 3) — 50% faster one bench
   up, 75% two up — is unblocked and untaken. `deploy::bench_near`
   answers a bool; it would have to answer "best rung in reach", and
   `craft::enqueue` would read it.
2. **The panel draws indents, not edges.** `ui/techtree.rs:49` says so in its
   own comment ("an indent (and one day a line)"); a line renderer between
   parent and child is cosmetic and waits for a real look at the screen.
3. **The operator has not seen it** — the tree panel, the two greybox
   benches, the tier badges. The visual gate is a person (`CLAUDE.md`);
   boot the game, stand at a bench, press `E`.


## 0tree · How deep the research tree goes, and the blueprint nobody can trade *(systems lane)*

1. **The tree's depth is still an unspoken pacing call.** It carries three
   edges now — `content/research.toml`: roadsign body behind medkit (:103),
   revolver and satchel behind gunpowder (:111, :116) — so the "one edge
   deep" reading is retired, and `DECISIONS.md` §open "research ladder v0"
   is stale in the same direction: it says revolver-behind-gunpowder is
   deliberately unauthored and `research.toml:116` authors it. What is open
   is how many more edges, over which bench tier now that workbench 2/3
   exist (§0tt). Do not invent one; fix the DECISIONS row when it is spoken.
2. **No blueprint ITEM**, so learning stays instant and personal and there
   is nothing to trade — the half that makes another player's progress
   interesting. Unbuilt, and it is a wire change
   (`crates/sim-core/src/research.rs` header records the omission).
3. **Nobody has seen the research/tech-tree panel work.** `ui/techtree.rs`
   and `render/panels/tech.rs` are gated headless only (`client/tests/ui.rs`
   §M); past `decode_event` nothing has been looked at. Same residual as
   §0tt item 3 — boot it, stand at a bench, press `E`.


## 0rs · Bodies are out of the raid storm *(systems lane)*

1. **Bodies are out of the storm.** `sim-core/tests/raid_storm.rs`'s own
   fixture sets the throwable's `damage` to 0 (`storm_combat`, line 212),
   deliberately — a blast that killed the players would measure a
   graveyard instead of a cap. The consequence is that `MAX_BACKPACKS`
   and the death/respawn ring are the one client-driven family the storm
   does not reach. The shipped charge does 475 and `charge.rs:526` hurts
   bodies, so the arithmetic exists; what is missing is a bounded gate
   that drives it at the tick's command ceiling without the run ending in
   a few ticks.
2. **The fleet raids ITSELF, and that is a design gap rather than a bug.**
   Attacker and owner never share a plot (`peak_shared_plot == 1`, measured),
   so every raid in the tree is an attacker blasting the foundation it laid
   four steps earlier — `raid_shape.rs:33` says it outright: *"a self-raid is
   a poor game and a perfectly good `EV_STRUCT_HIT`."* It does not stop the
   raid, so it is not the explanation for anything; it means no fixture in
   this tree has ever modelled two parties. `§0pop`'s `index % 2` owner/
   attacker split is the knob that would.


## 0rc · The wire raid's two unmeasured differences *(systems lane)*

1. **The tree contradicts itself about dropped actions.**
   `server/tests/raid_shape.rs:73` and `server/src/botclient.rs:399` both
   say `push_action` silently drops the rest, so a lost step 4 leaves step
   5 throwing at nothing. `server/src/core.rs:722` says the opposite — a
   deferred action stays ringed — and the code agrees: `net.rs:2054` pops
   the action ring only through an open hand, and the stream reader at
   `net.rs:1519` sleeps on a full ring rather than dropping. One of the
   two is wrong; the harness's "this leans optimistic" argument rests on
   it, so settle it before quoting that argument again.
2. **The jitter buffer's held-item timing.** `Client::consume_input`
   (`server/src/client.rs:576`) executes one buffered frame per tick, so
   the frame carrying `charge_slot` need not be in force when the throw
   lands. Cannot be the whole story: 27 charges did arm.


## 0r · A blast is silent and cannot be stopped *(systems + audio lanes)*

Offence landed (`sim-core/charge.rs`, `tests/blast.rs`, `DEATH_BY_CHARGE`).
What it still cannot do:

1. **No detonation sound and no detonation visual.** The `Cue` enum
   (`client/src/sound/mod.rs:96`) has no blast voice, and there is no
   `EV_BLAST` — the client learns of a blast only through `EV_STRUCT_HIT`
   and `EV_HEALTH`, so a near-miss is silent. Audio lane; wants either a
   cue keyed off the existing events or an event of its own.
2. **No dud chance and no defuse verb.** Stated in the tree at
   `sim-core/src/charge.rs:38` — a fuse that has started always detonates.
   Each is its own verb.


## 0aa · Building rights: the roster's third customer is missing *(systems lane)*

1. **No `AutoTurret`, so the roster has two customers and not three.**
   `sim-core/roster.rs` exists because the reference has four; ours has
   the lock's auth/guest lists and the hearth's crew. `grep -rni turret
   crates/ content/` returns only that header comment and one
   `ARCH_TURRET` example in a protocol doc line.

⚠ Three doc comments in `sim-core/{deploy,claim}.rs` still cite "§0aa
   item 1" / "items 1–2" under the section's OLD numbering; renumber or
   re-point them if this item moves.


## 5d · The agent player: the trust ledger is minted and nobody reads it *(systems lane)*

`PLAYERS.md` has the spec — verb set, observation encoder, four walls. Wall 3
is built (`EV_TRUST` code 39, `World::log_trust`, six checks in
`crates/sim-core/tests/event_roles.rs`); the other three are not.

Remains, in order:
- **Nothing reads it.** `ShardCore`'s drain ends `_ => {}`
  (`crates/server/src/core.rs:2465`) and no file under `crates/server/` names
  `EV_TRUST`, so no shard-hour is recorded until a server lane sinks it.
- **A dropped row is gone** — it rides the 256-seat drop-newest ring
  (`MAX_EVENTS_PER_TICK`, `limits.rs:624`), and unlike every other event a
  resync cannot re-derive a fact about a moment.
- `TRUST_GIVE` waits on the give verb; there is still no player-to-player give.
- Then the verb table, wall 1's subset gate in the same commit, then an agent
  client that plays badly. Entry price and earnings are `ALPHA.md`.


## 4 · A payload swap is still not a compile error

Every `EV_*` code carries a role check against a real cause and
`NOT_COVERED` is empty (`sim-core/tests/event_roles.rs`), with the seat kept
for the next code. What remains is not tests:

1. **The stronger form: a payload-role table both the emit site and the
   check read, so an a/b swap is a *compile* error** rather than a gated
   value (`reference/FINDINGS.md` §1 end). The gate says so itself
   (`event_roles.rs:3486`) and calls it a different shape of work — bigger
   than one pass.

⚠ The ledger is **40** codes now, not the 32 this heading claims
(`event_roles.rs:3498`); fix the number when you next touch the line.


## 0q · The gaps nobody has claimed

`crates/`/wire work no single-surface lane may take.

1. **The UDP buffer's ops half.** `net::bind_udp` asks 8 MiB and records
   what it got; this box grants 4 MiB (`rmem_max`). Raising the sysctl on
   the public shard is an operator act.
2. **Shore barrels as a second destination class.** The road pays unevenly
   and the haven pad is the one place worth walking to; a second class on
   the shore would give the ring two ends. Nothing else in this file
   mentions it.
3. **The wipe.** Named by both judges, described nowhere; `wipe-now` is in
   no crate. Economy half (`ALPHA.md` A1→A3) and operator half
   (`CLAUDE.md`), so the loop's share is the mechanism, never the trigger.
   Needs scoping before it can be an item. (`ALPHA.md` §Admin lane cites it
   as "§0q item 2" — one of the two numbers is wrong.)
4. **What the soak still owes.** Ticks, AOI and bytes are all measured
   (`DECISIONS.md` §open, the 100-bot baseline). Missing: tick jitter as a
   **distribution** rather than a threshold crossing, and an **hour** — the
   run was 25 minutes, so slow leaks are not excluded. Contention now has
   its instruments (`sim-core/tests/raid_storm.rs`, and `botclient.rs`
   drives `bots::raid_step` over the wire); nobody has re-run the soak with
   them.

⚠ Delete the duplicate "you cannot stand ON anything" and "the soak has
never been run" items — both landed and both contradict items above them.


## 0zd · Doors and locks — the key lock's blocker died and nobody re-took it *(systems lane)*

Locks landed whole 2026-08-08/09 (`sim-core/lock.rs`, `reference/DOORS.md`,
`DECISIONS.md` §open "lock v1") — boxes, the guest/pickup tier, the keypad
panel. One thing survives, and it survives because a **refusal outlived its
reason**.

1. ⚠ **The key lock was refused for a blocker that is now paid.** The stated
   cost was that keys need per-item instance data `ItemStack` has no room for;
   `ItemStack` gained `pub cond: u16` on 2026-08-16 with durability v0
   (`sim-core/src/gather.rs:532`), so instance data now runs through the
   inventory, the wire and the save — the four costs `DOORS.md` §9.7 named.
   The exclusion therefore rests on one unreviewed reason (the reference
   abandoned it in Devblog 193), which may well still be the right answer —
   but it has not been re-taken since the blocker was paid, and the false
   premise is written in **three** places: here, `reference/DOORS.md` §9.7 and
   the `DECISIONS.md` 2026-08-08 row. Correct all three or re-take the call.
2. **The knob registry contradicts the tree.** `DECISIONS.md` still declares
   *"Client: `L` opens a keypad **HUD line**, not a panel"* while the client
   ships `render/hud.rs::pad_overlay`. `CLAUDE.md` calls `DECISIONS.md`
   authoritative on every knob, so the registry is the thing to fix.

Still not owed, and this half stands: door tiers past wood and metal are a
content row, not a mechanic.


## Wire, shard and persistence *(server lane)*


## 0fan · The event lane's fan-out — four arms filtered, nineteen to go *(server lane)*

1. **Decide `EVENT_RING_CAP`.** Post-filter peak fan-in is `AOI_RANK_EXIT` = 64
   against a 64-slot ring — zero headroom, measured 50 of 64 under the worst
   fixture built (`snapshot_budget.rs` asserts the equality). Raise it (322 B a
   slot per connection) or batch a tick's events (`PROTO_VER` work). Operator's
   call; `DECISIONS.md` §open (event-lane fan-out v0) has the trade. The
   other half of the same number: **`EVENT_RING_CAP` (64) is smaller than
   `MAX_PLAYERS` (100)**, so 65 simultaneous swingers resync every client at
   once and a resync re-drips seven cursors.
2. ⚠ **Operator, a game question**: should the OWNER hear their own door
   knocked from anywhere on the island? Nothing has an owner check to hang it
   on (`server/src/core.rs` EV_KNOCK arm, `hud.rs`).
3. **The deploy walk is unaimed, and it blocks `EV_DOOR`/`EV_OVEN`** — `core.rs`
   streams `deploys.entries()` whole. Order: (a) aim `EV_DEPLOY_PLACED` the way
   `EV_PIECE_PLACED` is, (b) aim the walk on the same anchor, (c) then those two
   become filterable. `server/tests/deploy_wire.rs` pins the current truth and
   its counts go red when the seam is aimed. Sizing, so nobody re-derives it:
   the band is **3.2% of the island's area** against `MAX_DEPLOYS` 1024
   (`findings/swing-fanout-20260824.md`).
4. **The storm is combat only** — three arms of twenty-two. `raid_storm.rs`
   drives the other verbs at the command ceiling with nobody swinging; the two
   fixtures want merging.


## 0n1 · Class-S interest — the grid is still missing *(server lane)*

The radius filter landed 2026-08-18 (`crates/server/src/interest.rs`, gate
`crates/server/tests/piece_interest.rs`, `DECISIONS.md` §open "class-S
interest v0"). What remains, ranked.

1. **The grid.** No chunk version, no subscribe/unsubscribe, so no client
   can be told to forget a region — which is why removals stay broadcast
   and why a re-arm re-walks the in-range set instead of the difference.
   `NETCODE.md` §5/§7 proper, and it wants a wire change.
2. **Deploys and backpacks are unfiltered.** `server/src/core.rs:1975` says
   so outright ("the deploy walk is unaimed"), and the deployable walk still
   restarts on a removal (`deploy_sync_cursor = 0`, core.rs:2396); the
   backpack walk restarts on a loot or despawn (core.rs:2819).
   `reference/NETWORK.md` §9.2.1's amplifier, one store over.
3. `test_stream_in` (`NETCODE.md` §11) is still unbuilt — no `.rs` file in
   the tree mentions it. This gate counts records, not the client's
   per-frame apply/teardown budget, which is the other half.


## 0tx · The transport's three residuals *(server lane)*

Config and telemetry landed 2026-08-15 (`DECISIONS.md` §open "transport
truth v0"; gate `crates/server/tests/transport.rs`). Open, ranked:

1. **Nobody has run the A/B.** `cc = "bbr"` is selectable in `shard.toml`
   and untested against CUBIC on a real path. `net_congestion_events` is
   the reading. Wants a shard with real players, not loopback.
2. **The sysctl half of the socket buffer is ops and still owed.** The code
   asks 8 MiB; `net.core.rmem_max` on the public shard's box decides. The
   readback pair (`net_rcvbuf_asked` / `net_rcvbuf_bytes`) now says which,
   so check it before tuning anything else.
3. **No client-side telemetry.** All of the above is server-side. The HUD
   still has no loss/RTT source, and `crates/client/src/lib.rs:291` holds an
   `Arc<Connection>` it never asks anything — nothing under
   `crates/client/src/` calls `stats()` or `rtt()`.


## 0sp · The encoder is the tick's largest phase now *(server lane)*

`crates/server/src/bin/profile.rs` reports elapsed time and **must not become
a gate**; `valgrind --tool=callgrind` gives the per-function ranking.

1. **The encoder is now the largest phase** — ~0.43 ms of a 0.83 ms tick at
   100 clients in one AOI cell. Nothing else in the tick is close.
2. **`World::scatter_clear` still resolves cells cold per spawn pick**
   (`crates/sim-core/src/world.rs:1541` — three `terrain::scatter` calls per
   candidate, no `SlotCache`). It is **not** the crosshair's three-line fix:
   it is `&self`, and its 3×3 window *moves every candidate* along the spawn
   ring, so the cells are distinct and a memo only pays across repeated
   picks. Measure a respawn storm before threading `&mut self` through the
   picker.
3. The soak still owes tick jitter and real bytes (§0q item 4, `CLAUDE.md`
   wall 3's ⚠).


## 0y · Persistence — the three questions still open *(server lane)*

1. **A sleeper does not block movement.** Players never collided, so sleeping
   changed nothing; the question is unanswered rather than decided.
   Lootable-alive is item 1 of whatever comes after.
2. **The same-window rejoin.** A victim reconnecting in the very window that
   evicts them gets the store record fetched *before* the eviction save is
   filed — one window wide, the save ring's freshness class; the takeover
   hint already refuses to wake a condemned body (`server/core.rs:487`).
3. **Still no WAL, and the world file answered what a WAL would have
   forced**: a world load is an *origin*, not a command — the WAL header pins
   the origin hash beside the seed and the content hash, and replay starts
   there. `worldsave.rs`'s module header has the argument. Recorded because
   `§0ad2` item 4's set-time refusal leans on it.
4. **Still ungated:** the three-thread shutdown path end to end, and
   `KeySlot`'s id match (`server/net.rs:573`). Measured by hand only — a
   signal test is a clock test (`CLAUDE.md`): SIGTERM flushes and exits,
   SIGKILL leaves no `.tmp` and the next boot resumes off the last cadence
   save. Nothing in `crates/server/tests/` drives it.


## 0ad2 · What the admin lane still cannot do *(server lane)*

Six verbs plus `/bug` ship on the chat lane with no wire change, gated by
`server/tests/admin_wire.rs` (7) and `protocol::admin` (6). Open:

1. **A ban dies with the process.** `server/src/admin.rs:173`'s `Bans` is
   memory only (`net.rs:605` constructs it fresh). Persisting one wants its
   own file with its own format version — sharing the player store's header
   would wipe it on the next seed change.
2. **Nothing has typed a command against a live shard.** Every branch is
   gated headless; the socket half (`conn.close` with `REFUSE_ADMIN`,
   `net.rs:791`) has never been driven end to end, and the client has no
   dialog for it — `client/src/lib.rs:486` reads `refuse_text` at connect
   only.
3. **The anomaly log has no reader.** JSONL on purpose so `jq` is the
   reader, but nothing summarises a session, and the alpha gate's "zero
   silent failures" wants a verdict. (`ci/reports.py` is the `/bug` board,
   not this.)
4. **No `/who`, and no set-time** — the latter blocked by choice: day/night
   derives from the tick, so it wants the wire field §0y4 did not spend.
   ⚠ `/tp <id>` exists and the two-arg form is a decided refusal
   (`protocol/src/admin.rs`), so drop it from this list.


## 4b · The domain gate's one file-local residual

`SOURCES` (`protocol/src/event.rs:4351`) reads every `sim-core` module both
ways and every enumeration width is classified. One residual:

1. **`death_causes_are_a_closed_ledger` still scrapes `world.rs` alone.**
   `sim-core/tests/event_roles.rs:3704` opens with
   `include_str!("../src/world.rs")`, so its *contiguity* claim is
   file-local. Narrow, since the protocol gate catches a stray value
   crate-wide. `sim-core/tests/domain_ledger.rs` now applies the same
   scrape to three other families and is the pattern to follow.

⚠ This label collides with the world-lane §4b further down the file; give one
of the two a distinct label so a crate citation can name which.


## 0pop · The inhabitants nobody has run for longer than a test *(server lane)*

1. **Nobody has run one for longer than a test.** `DEFAULT_SHIFT_SECS = 300`
   (`crates/server/src/population.rs:47`) and `tests/population.rs`
   exercises ~0.2 s of it, so re-manning, the `RECONNECT_BACKOFF_MS` backoff
   and the shift report are gated only by construction. Cheapest next step:
   set `population = 8` in a real `shard.toml` (it is still commented out at
   `shard.toml.example:298`), run the shard, read the population line.
2. **Nobody has checked what an inhabitant can afford.** The shipped kit is
   a rock and a torch (`content/balance.toml` `[[spawn_kit]]`), while the
   raid rows a post plays are a fixture — the satchel is granted directly in
   `crates/server/tests/bot_smoke.rs`, never crafted. Judge -18 §B.2 is the
   live half of this.
3. Two proposed defaults stay open in `DECISIONS.md` §open ("shard
   population v0"): the 300 s shift and the 2 s backoff, plus what N an
   alpha shard should actually run and whether the owner/attacker split
   should stay `index % 2`.


## 5b · The wire still accepts two refusal reasons the sim can never mean *(server lane)*

⚠ **This section said CLOSED and covered half the problem.** The decode
narrowing landed for the bag `why` and the consume `reason` (`REFUSE_C_MAX`,
derived, `protocol/src/event.rs:365`). Two more domains are unbounded end to
end, and the tree names this section as their record: `sim-core/src/craft.rs:78`
says in its own source that the craft-refused subtype *"writes a full byte and
the wire bounds nothing, which `NOW.md` §5b already carries as the decode-side
gap."*

1. **Craft-refused.** Six reasons (`craft.rs` `REFUSE_RECIPE`..`REFUSE_BLUEPRINT`,
   1..=5) and deliberately **no `REFUSE_C_MAX`** — the name is taken by
   `survival.rs`'s consume refusals, whose prefix the domain gate scans
   crate-wide, so the obvious constant would collide. That is a naming problem
   wearing a wire problem's clothes; pick a name the scanner distinguishes.
2. **Deploy-refused.** `deploy.rs:314-318` declares `REFUSE_D_KIND`..
   `REFUSE_D_REACH` and there is **no `REFUSE_D_MAX` anywhere in the tree**.
3. Neither appears in `event.rs`'s `DOMAINS` table, so
   `every_domain_fits_its_wire_field` cannot see them and the encode site
   bounds nothing.

No `PROTO_VER` turn is owed — this is the narrowing rule at `PROTO_VER`
(`protocol/src/lib.rs`), the same judgement `5c` already made.


## The frame, the screens and the client's own hot path *(client lane)*


## 0fill · The darks, second half: the transfer *(client lane)*

The hemisphere fill landed (`render/fill.rs`, `tests/fill.rs`); the transfer
did not. Cast shadow on *open* ground is an up-facing surface and no
hemisphere darkens it, so the measured p10 is untouched (79.9 against
`ART.md` §3's 49).

- The rig's floor arithmetic is written in the wrong space. `rig.rs` sets
  `fill = 0.30 × sun_on_flat` for rule 3's "shaded ≥ 0.30 of lit", but the
  delivered *linear* ratio is 0.229 — under the floor it aims at — while the
  judge measures 0.725 in *display* luma. Rule 3 is a pixel ratio and the
  constant is an illuminance one; both readings cannot be acted on at once.
- So the lever is the tone curve, not the fill: `Tonemapping::TonyMcMapface`
  (`rig.rs:212`) plus `Exposure { ev100: 14.2 }` (:206, 0.8 stop off
  `SUNLIGHT`) is what puts 0.229 linear at 0.725 display.
- **Do not do this blind.** It is the coupled set (`CLAUDE.md`: three
  parallel passes 60→66, one sequential owner → 26) and the last correction
  overshot. One owner, one iteration, with the frame open.
⚠ **And the next pass to capture must know the tonal baseline moved.** Every
`-visual.md` in `findings/` predates `rig::DayPin`, so it was shot at a
24–27° sun against the pinned noon a capture run now takes. Its luma, sky and
shadow numbers are **not comparable** to the next report's — do not read a
brightness delta there as the effect of a render change.

- Blocked on a *capability*, not priority: a pass that can capture should
  take it before anything below. §0gp item 1 (8.0% mean luma) and item 3b
  (`reflectance: 0.18` → F0 0.52%) are debts against this same owner.


## 0gc · A blade shaded exactly like the dirt it stood in — LANDED *(client lane)*

**Landed 2026-08-25.** `Soup::tri_ramp` takes the blend as a function of the
vertex; `blade()` ramps 1.0 at the root (the ground's normal, so `ART.md`
rule 2 keeps the blade bedded) to `BLADE_TIP_BLEND = 0.75` at the tip. `tri`
delegates to it, so none of its twenty-odd other call sites moved. Gate:
`tests/contact.rs::a_blade_separates_from_the_ground_it_grows_out_of`, red on
the shipped value, where it prints what it was — **tip normal y = 0.9978**,
the ground's normal to three decimals. Knob: `DECISIONS.md` §open, clutter
contact v0.

⚠ **This item carried TWO false mechanisms and both are dead.** The winding
claim went to `a_blades_two_triangles_do_not_wind_opposite_ways` (dot > 0.99
over a 128-case sweep). The `double_sided` claim — that Bevy negates the
shading normal on a back-facing fragment — was checked against Bevy 0.18.1 on
2026-08-25 and is **also false**: `pbr_functions.wgsl:130-134` guards that
negation with `#ifndef VERTEX_TANGENTS`, `mesh.rs:2410` defines
`VERTEX_TANGENTS` whenever the layout carries `ATTRIBUTE_TANGENT`, and
`Soup::mesh` calls `generate_tangents()` on every clutter tile. **No blade is
ever flipped.** Do not turn `double_sided` off — it changes nothing here and
would black out the real back faces.

What is left is the one number: **0.75 is invented and nobody has judged it**,
against `ART.md` §5's "blades catch a rim of sun at their tips".


## 0gp · The ground splat's residuals: a projection, a specular, and five prop maps *(client lane)*

1. **Still planar XZ, not biplanar** — a vertical face stretches;
   `assets/shaders/ground_splat.wgsl` projects no other way (`RENDER.md` R4).
2. ~~**`reflectance: 0.18` → F0 = 0.52%**~~ **LANDED 2026-08-25, and it was
   not one constant**: *every* material in the client was authored 8–70× under
   physical, because Bevy's `reflectance` is a remap (`F0 = 0.16 × r²`) whose
   default 0.5 already IS the dielectric 4%. One owner now,
   `render/fresnel.rs`; `tests/fresnel.rs` reads every prop material back out of
   the asset store and fails anything outside 1.5–6% F0 (red on the shipped
   `bark` 0.08 = 0.10%). The ordering this item insisted on held — the
   per-texel field landed first, so it is turned up over a field and not a
   scalar. ⚠ **The −0.4% roughness null result has NOT been re-measured** with
   energy in the lobe. `DECISIONS.md` §open "specular v0".
2b. ~~The four ground `*_ao.jpg` are read by nothing~~ **LANDED 2026-08-25** —
   bindings 114–117, blended by the same `bw` as colour, normal and roughness,
   folded into `diffuse_occlusion` with `min` per `ART.md` §4 (never a
   multiply: two occlusion terms of one scale double-darken). Diffuse only.
   The binding gate now scrapes BOTH the WGSL and the Rust struct — it held the
   shader against a hand-kept list that *claimed* to be the struct and never
   read it. ⚠ Nothing in this repo compiles the WGSL; a syntax error there is
   green in CI and dead at boot. **Booted 2026-08-26 under lavapipe: it draws.**
3. **`ground_detail.jpg` is loaded by nothing** — `textures::GROUND_DETAIL` has
   no load site; the shader derives the field from `grass_albedo`. Deleting it
   is a separate call: a pre-baked field is what a cheaper LOD would want.
4. **Operator:** granite passed beach sand and the minimap's `ROCK` did not
   follow; fixing it departs from a `mapraw.jpg` reading (`DECISIONS.md` §open
   "minimap palette v0"; `client/tests/map_palette.rs` pins it by name).
5. **The five PROP roughness maps are unread and `render/props.rs`'s reason is
   false** — Bevy multiplies `metallic` (default 0.0) by the map's B channel.
   It needs a LEVEL call: the map whole loses the authored `rock 0.88` /
   `ore_stone 0.80` split; mean-placing wants 1.44 and Bevy clamps at 1.0.


## 0gi · What the island still cannot show: no occluder at blade scale *(client lane)*

Items 1–3 are struck and gated (`sim-core/tests/relief.rs`,
`client/tests/ground_where_the_green_goes.rs`, `client/tests/daynight.rs`).

4. **An occluder at blade scale is missing.** SSAO is enabled and this item
   was twice wrong about it: it is at `rig.rs:284`, at Medium,
   and "no SSAO anywhere" is stale. The paint read is `clutter.rs`'s NORMALS
   (§0gc). What nothing pays is `ART.md` rule 2 for the tile — the clutter
   mesh carries `NotShadowCaster` (`clutter.rs:504`) and a blade's dark base
   darkens the blade, never the ground under it.
5. **Litter still wins every mix, and this item's numbers are stale.** After
   §0gp's albedo re-place, recomputing off `terrain_mesh::GROUND_ALBEDO`
   gives litter **2.49×** grass's value (not 3.2×) and grass must hold
   **≥78.0%** of a grass/litter blend to read green-dominant (not 82.1%).
   The gate (`ground_where_the_green_goes.rs`
   `grass_must_hold_most_of_a_mix_for_the_ground_to_read_green`) asserts
   only `> 0.66` and `> 2.0×`, so it stayed green through the drift. The
   mosaic is not itself a defect; the boundary still never reads as grass.


## 0w · The props' remaining gaps — darks, density, unread roughness *(client lane)*

1. **The p10 gap, still the top visual one** — 71.0 against a reference 41.0
   (`RENDER.md` §0). The hemisphere fill landed (`render/fill.rs`,
   `tests/fill.rs`) and bought direction, not the p10; the transfer half is
   what is left (`RENDER.md` §5 item 6). One owner in the coupled set.
2. **Trees are small and sparse in the midground** — an empty green plain
   between near clutter and far ridge. `terrain::scatter`'s density and the
   conifer's scale, not a material; the same ceiling §0t item 2 prices.
3. **The dirt skirt is nobody's.** `props::SINK_M` (0.06 m) sinks every prop
   and `tests/greybox.rs` evaluates "nothing floats"; crowding where a
   boulder meets turf is still missing (`ART.md` rule 2).
4. **The far mesh speckles.** Grazing-angle aliasing on the 8 m LOD;
   `textures.rs` pins `anisotropy_clamp: 4` for a browser reason that did not
   survive the port (`ART.md` §7) — a proposal, not an edit.
5. **Roughness maps unread — ten now** (`assets/textures/*_rough.jpg`).
   Blocked on an ORM packing step: `metallic_roughness_texture` is
   glTF-packed and its B channel is metallic (`render/props.rs:1090`).


## 0out · The horizon has trees — what the outer ring owes *(client lane)*

Landed 2026-08-25. `props::OUTER_RADIUS = 5` streams an 11×11 chunk ring of
TREE-ONLY hulls past `NEAR_RADIUS`, one entity each (`spawn_outer_tree`) — no
`Topple`, no stump, no canopy, no `VisibilityRange`. ~1,260 trees at 105 tris
= ~132 k against `DESIGN.md` §9's 1.5 M. The radius it replaces was sized when
a tree cost 5,900 triangles and never re-derived after `impostor_of` made it
105. Planted on `terrain_mesh::far_ground_y`, not `slot.y`: the ground drawn
out there is the 8 m far mesh minus `FAR_DROP`, measured **0.630 m** off the
heightfield at worst. Gates: `tests/outer_ring.rs` (4); one mutant caught a
worthless assertion in the first draft.

1. **The hull is untextured** — it wears `foliage` (white, vertex-coloured, no
   map), so the midground is flat green shapes and this ring multiplied them by
   four. `WANTED.md` §9.5's leaf texture is the cheapest fix and serves the
   bush too. **Highest-value item here.**
2. **The harvest sweep got denser and that was a named cost.**
   `harvest_changed` measured 1,500 props × a full 16,384 set at 2.34 ms and
   warned that a denser ring is the case that worsens. Outer hulls carry
   `Fellable` for correctness, so the count roughly doubles on frames where the
   harvested set moves. The real fix is that `HarvestedSet::contains` is a
   linear scan. Unmeasured on a GPU.
3. **Only trees.** Boulders and barrels still stop at `NEAR_RADIUS` — a
   sub-pixel lump costs an entity and changes no silhouette.


## 0t · the forest — what it still owes *(client lane)*

1. **The broadleaf has never been LOOKED at.** `SPECIES` is two rows, pool 6;
   every check on it is arithmetic. Boot it and look — likely wrong are
   `children`/`angle[1]` and leaf `count`/`size`; `PLANTS.md` §3.1 has
   ez-tree's 15 presets to take real numbers from. A species is a row.
2. **The density ceiling** — one occupant per 8 m `CELL_SIZE` cell.
   `PLANTS.md` §3.2 prices the three ways up; all sim-core, none cheap, the
   cheapest (`CELL_SIZE` 8 → 4) quadruples live `SlotLives` rows against
   `TERRAIN.md` §6's budget. Not a rendering change.
3. **The billboard LOD is optional now, not owed** — `impostor_of`'s 105-tri
   hull took the p90 ring 1.94 M → 510 k, under `DESIGN.md` §9's 1.5 M.
   `TERRAIN.md` §4's octahedral billboard is the cheaper end, still unbuilt.
4. **`aWind`** — `StandardMaterial` cannot read a custom attribute, so wind
   needs the custom material `RENDER.md` lists. Gets LOD1 for free.
5. **Sub-canopy empty, shrub layer one blob** (`Occupant::Bush`, `PLANTS.md`
   §2): ez-tree's `bush_*` presets and a small tree at 40 % are new
   `Occupant` variants plus scatter rows.
6. **The needle card is generated** (`tree::needle_image`); `WANTED.md` §9.5
   is the swap, the highest-value texture on that page.


## 0a · The clutter ring still ends on a line *(client lane)*

`render/clutter.rs` has no distance term: `CLUTTER_RING = 2` over
`CLUTTER_TILE_M = 16.0` puts a hard edge at ~32–45 m. Two findings stand:

1. **The fade's recipe**, already proven at the other boundary
   (`sim-core/terrain.rs::swept_here` cites this item): thin
   stochastically by instance hash so the same elements survive at a given
   range, then scale the survivors to zero. Whether the edge reads at all
   at that distance is a question for a person with the game booted, not
   for a guess.
2. **Beach skirts are thin because of the scatter table, not the skirt
   path** — ~0.22 prop centres a tile on the coast against ~0.95 inland.
   ⚠ Neither ratio is in the tree; both are browser-era measurements and
   want re-measuring against `terrain::scatter` before they are acted on.


## 0y · The sea is a volume — what it still cannot do *(client lane)*

1. **The last hard edge needs the depth prepass.** The alpha ramp is a
   *vertex* quantity off `terrain::height`, so it rings against a
   boulder, a foundation or a player in the shallows. Sample the prepass
   in the fragment, fade alpha as scene depth nears the water's own.
   Needs an `ExtendedMaterial` and WGSL (`RENDER.md` §8); both already
   exist in the tree (`assets/shaders/ground_splat.wgsl`), and **the third
   input exists too** — SSAO already puts a `DepthPrepass` on the camera, so
   the fragment has something to sample.
2. **One sea state, no weather.** A storm is `WAVES` scaled by a scalar
   the sim would have to publish — wire, not renderer.
3. **Nothing reflects.** `reference/WATER.md` §5/§6 first: reflections
   are the expensive half and the payoff is the sky.
4. **Underwater is audio-only.** A colour grade under the surface is a
   second owner of the frame's haze; it wants the lighting owner.
5. **The submerged duck is not a filter** — rodio gives gain, rate and
   panning; a real low-pass needs a DSP node.
6. **`Splash` is the only producer of the waterline** — no stroke, no
   wake, no interactive deformation.


## 1 · The native pivot — the one visual gap left of it

R0–R6 and R8 all landed and the browser client is deleted. What survives:

1. **Cloud form.** The deck reads stratus where `ART.md` §4 asks for
   cumulus; the p90 gap is 25 luma. `RENDER.md` §8 ranks it second, behind
   the gate-asserts item, and no other section in this file owns it.

⚠ Drop the rest of this section's list. The hemisphere fill landed
2026-08-15 (`render/fill.rs`, `client/tests/fill.rs`) and the four-way
splat landed the same day (`render/ground_splat.rs`,
`client/tests/ground_splat.rs` — four identities, four photographs), so
neither is a gap. The depot is published; the republish that is still owed
is §0win's, not this item's.


## 0chr · The clips the wire cannot yet ask for *(client lane)*

1. **The clips outrun the states that would play them.** `interp::RemoteState`
   carries only id/pos/yaw/pitch/live/sleeping/dead, so `Jump_Loop`,
   `Swim_Fwd_Loop` and the crouch pair sit unplayable in `stumpy.glb` — each
   needs a fact on the wire. Crouch is an input bit the sim ignores
   (`render/input.rs:289`) and never reaches a snapshot.
2. **The gather swing is `Sword_Attack`** (operator, 2026-08-17): no asset is
   owed, and the blocker is item 1.
3. **The item is not parented to the hand** — `render/viewmodel.rs:572` says so
   in its own comment; `ViewArms::hand` holds the bone, and the grip offset and
   tilt need re-deriving against the arm's frame, which is judged by looking.
4. **No render layer**, so the arms and the held item can clip into a wall: a
   second camera would duplicate the exposure/tonemap/atmosphere owner.
5. **The head's pitch is clamped, not distributed.** `ANIM_HEAD_PITCH_MAX`
   is 0.9 rad and `anim.rs:811` says the remainder is dropped rather than
   spread — "distributing it across the spine is the follow-up this constant
   exists to make obvious." A steep look up or down bends nothing below
   the neck.
6. **The hand reads large and the fingers splay** — 24 joints, no finger bones,
   so a re-import or a sculpted grip is the only lever.
7. **Unlooked-at**: `Death01` on a real body, the collapsed off arm, the
   sleeper tint. ⚠ "No frame has a body in it" is stale — `render/capture.rs`'s
   scene pass shoots `7-player.png`, staged by `ci/scene.sh`.


## 0hand · Four items still draw the generic stand-in *(client lane)*

1. **Metal hatchet/pickaxe/spear** — no asset (`assets/models/WANTED.md` §5.6,
   while `content/items.toml` already ships `item.hatchet_metal` and
   `item.pickaxe_metal`), and reusing the stone glb would need a second
   material to not lie about the head.
2. **Fire pit** — `assets/models/deploy/fire.glb` bakes a LIT emissive, so a
   carried unlit one would glow and `held_assets.rs::nothing_held_glows`
   refuses it. Needs an unlit variant or a generated `heldgen` row.
3. **Resources, ammo, bandage, lock** — no models; the stand-in covers them.
   `ui::hold::HELD_MODELS` is 14 rows and none of them is these.
4. **The swing still pitches the item and not the arm**, so a mid-swing frame
   shows the fist behind the arc. Same fix as §0chr: parent the item to
   `ViewArms::hand` (`render/viewmodel.rs:572` records why it has not been),
   retune the grip against the arm's frame, then look at it.


## 0dur · Durability: the words, the wearers, the bench *(client lane)*

1. The pip is drawn in all four cells that hold a stack, but **the detail
   pane still says nothing in words** — `render/panels/craft.rs::build_detail`
   never reads `cond`.
2. **Weapons and armour do not wear.** `sim-core/src/combat.rs` says so in as
   many words, `condition_loss` rows exist only in `content/gatherables.toml`,
   and there is no `sim-core/src/armor.rs`. `reference/DURABILITY.md` §5 left
   both unsourced (per shot / when hit), so this is a research row, not a
   build item, and wear-on-swing-at-players is a mechanism question (`tools as
   weapons`, `DECISIONS.md` §open).
3. **Repair is v1 by decision** (Q3: re-craft is the repair). When a bench
   lands it is `Station::Workbench1..3` (`content/src/schema.rs`) plus a
   blueprint check, never a new deployable, and `DURABILITY.md` §3's 0.20
   ratio stays DISPUTED until someone checks it against the in-game price.


## 0ps · Pieces: staged damage, the missing shapes, the repeated wall *(client lane)*

1. **Damage bands have never been staged**: one row of one material, hit a
   known number of times, photographed at each band. ⚠ §0mk — no decal
   renders under lavapipe, so a headless run cannot check marked surfaces.
2. **11 shapes against the reference's 20** (`BUILDING.md` §7b.1):
   `sim-core/src/build.rs` declares `SHAPE_FOUNDATION`..`SHAPE_TRI_ROOF` only
   — no half/low wall, floor frame, steps, ramp, 3 of 4 stairs. Rule 6 is
   silhouette before surface, so this outranks more material work.
3. **A base is a hundred identical walls at one rotation** (rule 7).
   `render/structures.rs` sets `uv_transform` from the tier's scale alone; the
   fix is a pool of per-tier variants (offset + tint) by address hash.
4. **Trim** — lashings, plank seams, a capstone rim; `shape_parts` is the
   place, but price the entity count first at `MAX_PIECES` 8192.
5. **Deployables got the wire fix, no damage visual** — the deploy material
   takes no `hurt` term where a piece does — and nothing shows which face was
   struck.
6. **Roughness maps still unwired**: scalar `perceptual_roughness` only. An
   ORM packing step would serve terrain+props+pieces at once.


## 0u · Stairs are a plate, and a lock cannot be aimed at a door *(client lane)*

1. **Stairs are still a flat pitched slab** in both the ghost and the standing
   piece — a ramp drawn as a plate, with no steps in it. Shared between the
   two, so at least they agree, and `sim-core/tests/base_lattice.rs` holds the
   tread a player walks to the ramp the sim walks. This is the SHAPE being
   undetailed, not `§0ps` item 2's missing stair variants.
2. **A lock aimed at a DOOR is unreachable.** `ui::place::deploy_target`
   special-cases `PLACE_DOORWAY` only, so `PLACE_DOOR` — the code lock's
   placement class (`content/deployables.toml`) — falls through to
   `SHAPE_FOUNDATION` at level 0 and targets the plane. On a box the `L`
   verb works. Noted at the call site, not built.


## 0x · The client makes sound — what it cannot yet hear *(client lane)*

1. **Nobody has heard it and nothing scores it** — `ART.md` has no audio
   section and this box has no device. `cargo run -p client --bin soundbank
   -- <dir>` writes every cue to WAV; sourcing is `assets/sound/WANTED.md`.
2. **The score is programmer art.** `synth::score` generates the nine
   `music::PIECES`; swapping in recorded pieces is one function
   (`synth::render`'s music arm). Two bumps we cannot take: weapon equipped,
   projectile near-miss.
3. **The `--capture` run is still by hand** and is the only proof most audio
   systems execute. `tests/music.rs` is the cheaper shape — any audio system
   with no world in its arguments could be gated that way.
4. **Two cues have no producer:** `ImpactWood`/`ImpactMetal` need to know
   WHAT was hit, and `UiClick` appears only as the mixer's placeholder
   `Request` — it wants a hook in the per-screen click handlers.
5. **No occlusion**; the prerequisite is a geometry query, and the correct
   one is the sim's (`collide.rs`), not a raycast against render meshes.
6. **Crickets** are a content-free companion pass — a night-gated `Cue`, the
   bird layer's shape with the predicate inverted (`render/audio.rs:672`).


## 0x · The native client — the feature trim and the dropped anchors *(client lane)*

1. **Trim Bevy's default features — with a verified build, not a guess.**
   `crates/client/Cargo.toml` still takes bevy with defaults on. Unused by
   grep: `bevy_gilrs` (no `Gamepad` anywhere — the one real system-dep win,
   `libudev`) and `vorbis` (the bank is WAV we generate). Load-bearing:
   `bevy_audio`, `bevy_gltf`/`bevy_animation`, x11 and wayland. Attempted
   2026-08-06 and backed out on disk, not code — and a green compile is not
   evidence: Bevy answers a missing decoder with a white fallback. Wants
   headroom and a `--capture` run someone looks at.
2. **World-space anchors are still dropped.** The HUD half landed
   (`hud::readout` pins the struct-hit fraction and the charge clock under
   the toast); the wall's own number at the wall itself and a clock on the
   charge mesh are not built, `charge_deploy` stays unread, and `stock_addr`
   is set in `client-core` and read nowhere under `crates/client/src`, so
   nothing says WHICH hearth. None is blocked.


## 0z · The Bevy-draws rule's missing gate *(client lane)*

1. **R-G4 is still the missing half of the Bevy-draws rule.** Placement has a
   gate; the no-gameplay-state-in-the-ECS rule has none. Its answer is the
   renderer-attached/detached state-hash equality (`RENDER.md` §5, line 889),
   and nothing under `crates/client/tests/` compares a state hash.
2. **Nothing photographs the wait.** A capture run exercises it and
   `render/capture.rs::PLACE_FRAMES` (300) bounds it; *seeing* it is §0p2
   item 3's viewer, which is also unbuilt.


## 0v · Players are people — what the rig still cannot say *(client lane)*

1. **Crouch, jump and swim are wired to nothing.** The clips are in the
   file; the snapshot carries no grounded bit on a remote body and no
   crouch bit, so `BodyAnim` cannot see them — `render/audio.rs::
   remote_steps` names the same gap. A protocol change (wall 6: version
   bump + regenerated goldens in one commit), not a client one.
2. **Nobody holds anything.** The viewmodel is first-person only; a
   remote mannequin has empty hands (`render/bodies.rs` spawns no held
   mesh). The rig has hand joints, so this is an attachment to a named
   joint rather than new art.
3. **Root motion is ignored.** The `_RM` variants are unreferenced in
   `crates/client/src`, so feet slide at speeds between the clips'
   authored ones — the fix is scaling playback rate to speed, a knob
   nobody has measured.
4. **A plain worn-steel albedo is the missing texture.** The axe head
   carries no map; the only metal in `assets/` is ribbed corrugated
   sheet (`render/viewmodel.rs`, `assets/textures/MANIFEST.md`).


## 0p2 · What the UI still owes *(client lane)*

1. **Rotate is still not a verb** — and the piece HAS a facing now
   (`PieceRec::facing`, hard/soft v0), so the asymmetry it waited on exists.
   `ACTION_SUB_BITS` is 5 and `ACT_MAX` is 18: the lane holds it.
2. **The hammer wheel's centre readout names the verb, not the target or the
   upgrade's cost** (`panels/wheel.rs`, `hammer::label`/`blurb`). Filling it
   wants `verbs::Near` at draw time, which `panels::rebuild` does not take.
3. **Nothing here can photograph a panel.** `render/panels/` (3,540 lines) is
   unreachable from `--capture`. Wanted: a **viewer, not a gate** — open each
   panel against a stocked fixture, write a PNG per screen, assert nothing.
4. **Fourteen distinct font sizes is not a scale** (`font`/`font_bold` sites
   in `render/`). Collapsing to five may not be done blind: they were
   budgeted against 720p and the first cut clipped a column at both ends.
5. **Surveyed and refused, do not re-survey:** `bevy_hui`, `bevy_lunex`,
   `bevy_feathers` (~5,400 lines of screens into a data-driven plugin) and
   the freegameui.net MCP (403s here, bypasses `bake_icons.py` and
   `tests/ui.rs` §G, pre-coloured kits fight tint-at-draw).


## 0w · The native menus — the rail and the untested gesture *(client lane)*

1. **The rail is not the reference's, and one wire field would fix it.**
   `EventMsg::Catalog` ships display names only, so a category rail by item
   class is not computable client-side (`ui/craft.rs:14-28`). A class byte
   per item, a `PROTO_VER` bump and regenerated goldens in the same commit
   (wall 6) buys the frame's real rail. Today's buckets are honest but they
   are not that.
2. **The drag is gated as arithmetic, not as a gesture.** `tests/ui.rs` §B
   holds the split arithmetic (`a_half_drag_sends_half`); press → ghost →
   release → send against a live shard is verified by inspection only.


## 0v · The menu flow — the served list and the untested hangup *(client lane)*

1. **Nothing re-checks that the SERVED shard document matches `shards.toml`.**
   `ci/shardlist.py --self-test` is a pure generator by design — no network,
   which is what lets it run in `ci/gates.sh` — so the diff between what it
   produces and what `GET /api/launcher/servers/gates` actually returns is a
   command somebody runs. The three days the list was served and dark
   (2026-08-20 → 08-23) are what that costs; `ops/certbot-deploy-hook.sh`
   now refuses a chain that does not cover the published name, which closes
   the certificate half only.
2. **Ungated, by hand only:** the end-to-end kill-the-shard-mid-play run
   behind `Screen::Disconnected`. Nothing under `crates/client/tests/`
   enters that state except `report_key.rs`'s key check.


## 0pw · Skinned meshes still specialize on arrival *(client lane)*

`render/prewarm.rs` warms every `StandardMaterial` off `AssetEvent::Added`.
What it does not warm is named in the module (lines 52–57):

1. **Skinned meshes are a different pipeline key** — a body's skin is a
   `SkinnedMesh` component, so the first remote player to walk into view still
   specializes on arrival, and the native symptom is a pop, not a hitch.
2. **The measure has no gate.** `PipelineCache::pipelines()` is public but
   lives in the render world and needs a GPU, so the count stays unasserted;
   `crates/client/tests/prewarm.rs` gates only what reaches the ECS.


## 0pf · The client's CPU frame — four measured leftovers *(client lane)*

1. **`ground_slope`'s four taps are ~80% of what a tile now spends** and the
   stencil is not takeable — it moves every splat byte, so it is a design
   change with a golden behind it, not an optimisation.
2. **`water::animate` clones ~677 KiB into the render world every frame** —
   `Assets::get_mut` deep-clones a `MAIN_WORLD` mesh on modification. Measured
   by `crates/client/examples/frame_cost.rs`: 7,921 vertices / 676.6 KiB,
   stream+animate 0.69 ms on a still frame. The fix is the vertex shader
   `render/water.rs` §57 names, and no `.wgsl` exists in the tree yet. Nothing
   on this box runs WGSL, so do it AFTER someone can boot on a GPU.
3. **Per-frame leftovers, measured and small** (under 50 µs together):
   `verbs::resolve` scans the piece mirror twice a frame and wants the 3×3
   `ColIndex` neighbourhood; `bodies::stream`/`mobs::stream` re-find each
   interpolator slot by linear scan after `ids()` knew it; `audio::fell`
   fetches a `GlobalTransform` to test `is_changed()`; `hud::update` rebuilds
   its strings; the ring streamers probe a full map every frame.
4. **The sea's tangent `w` is `-1` (`water.rs:947`), the ground's is `+1`
   (`terrain_mesh.rs:711`)**, same planar XZ set. One flips the ripple map's
   green channel — boot the game and look, do not guess.


## 0u · the frame budgets are browser numbers and nobody has re-derived them

`DESIGN.md` §9's budgets were set for a WebGL page and three no longer
describe what constrains us. The docs now say so; the measurement is still
owed and no gate or knob has moved.

1. **< 300 draw calls / < 1.5 M tris are WebGL-shaped.** Two shipped numbers
   are rationed against the 1.5 M: `CLUTTER_RICH_PER_TILE = 96`
   (`sim-core/terrain.rs:2919`) and the conifer ring's 1.9 M verdict.
2. **Nothing measures the native cost.** No `RenderDiagnosticsPlugin` in the
   tree, and no VRAM or disk figure anywhere. Capture on a real GPU at the
   ring's p90 tree count, read draw calls and frame time (its wall-clock
   half is not assertable — `CLAUDE.md`), and propose into `DECISIONS.md`
   §open. Renumbering is spoken, never taken by the loop.
3. **`BASE_ANISOTROPY_MAX = 4`** was chosen for a software-rasterizer reason
   that does not transfer. ⚠ It is no longer a constant — only a comment at
   `client/src/render/textures.rs:60`.

Initial load < 15 MB and `ART.md` §7's 12 MB payload are already retired in
the docs; 60 fps on a mid laptop iGPU survives as a hardware floor.


## 0p3 · Photographing a panel — the screen the recipe cannot reach *(client lane)*

The site recipe stays (two `DECISIONS.md` §open rows cite it):
`terrain::haven(seed)` / `haven_shelter` / `waystation_canopy` give the
coordinates, `shard.toml`'s `dev_spawn = "x,z"` stands the capture camera
there, then `Xvfb :9 -screen 0 1280x720x24 &`, the shard, and
`VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json DISPLAY=:9
WGPU_BACKEND=vulkan target/release/gates --server 127.0.0.1:4433
--capture <dir>` — six vantages, ~40 s. They face N/E/S/W, so stand on the
opposite side of what you want in frame. **This asserts nothing and must not
become a gate** (`CLAUDE.md`: the visual gate is a person).

Still owed, from §0p2 item 3: **the panels.** `render/capture.rs` knows two
subjects, `Player` and `Build`, and names no panel anywhere, so inventory,
crafting and the wheel are seen only by a human with a shard up. Wanted is a
viewer that opens each panel against a stocked fixture and writes a PNG per
screen — the camera pointed at a screen rather than at a place.


## 0vj · The capture probe ships frames with no record of what went wrong *(harness lane)*

⚠ The old premise is stale and should not be re-queued: the shell wrapper
landed outside this repo. `gates-loop/art/capture-native.sh` is what shot the
2026-08-14 frames (`CLAUDE.md` §the loop, `RENDER.md` §capture, and this file's
own §0gi item citing `capture-native.sh:44`), and a `-visual.md` was written.
Whether the loop is running at all is `CLAUDE.md`'s business, not an item here.

What is still open, and is ours rather than the harness's: the probe writes
PNGs only — `crates/client/src/render/capture.rs` joins four
`{idx}-{label}.png` paths and contains no `manifest` and no json — while the
visual judge's prompt asks for a `manifest.json` carrying the run's errors. A
capture that reports what the client logged while shooting is better evidence
than six pictures alone.


## 0bd · The tree blocks 0.3 m of ceiling nobody draws *(client+sim lane)*

The barrel was measured and closed (0.585 ⌀ × 0.88, gated both ways by
`greybox.rs::every_drawn_archetype_fits_the_volume_the_sim_blocks`). The tree
row is the one `greybox.rs` **excuses**, and it is only half closed.

1. **`OCCUPANT_TOP_M[Tree] = 5.7` cites dead code.** `terrain.rs:4245` reads
   `5.7, // Tree — PINE_TRUNK_H`, and `PINE_TRUNK_H` (`render/props.rs:45`)
   belongs to `pine_mesh`, which carries
   `#[allow(dead_code, reason = "the far-LOD silhouette, per TERRAIN.md §4")]`
   — nobody draws it. The drawn broadleaf is `SPECIES[1].height_m = 5.4`
   (`render/tree.rs:127`), so the sim blocks 0.3 m of invisible ceiling over
   half the pool. Nothing measures it: `greybox.rs:210`'s excuse holds only
   the **trunk radius**, by name.
   The fix is the barrel's — bound the drawn mesh over its own height band in
   `tests/tree.rs` and take the number off it, rather than pasting one.
2. **`assets/models/WANTED.md` §2.8 still briefs the loot barrel at the
   retired browser guess `0.9 ⌀ × 0.95`.** A mesh bought to that spec reddens
   the greybox gate on arrival, which is a purchase this repo would pay for.


## Numbers, worldgen and the arc *(content + world lanes)*


## 0b · Balance — the reference rows still outstanding *(content lane)*

⚠ **Derive the raid ratio, never quote it** — `Content::load_dir(…)` then
`.anchors()`, five lines. Four quoted readings have gone stale in two days.
⚠ Two operator rules (2026-08-10): a band of ours yields to a number of
theirs by default (`BALANCE.md` §6.5), and a number ABSENT from
`RIPLIST.md` has not thereby been decided either.

`reference/RIPLIST.md` §2 is the queue and the six steps; read it before
touching a balance number and do not re-derive the list here.

1. Next unblocked row is **1g**, the research ladder's per-item ordering
   (`READY`, page tier). Settle the era question (§1f) before taking it.
2. Blocked, researched, numbers already written down: **1j** `armor.toml` —
   one re-anchor of `content/tests/content.rs::band_breaks_refused`, best
   landed inside equipment v0; **1i** `loot.toml` — needs a `guaranteed`
   column on `LootEntry`, and the half-take measures 9× worse than nothing.
3. **No per-material damage resistance**: `content/src/schema.rs:281` has one
   `structure` column, so the ladder above stone is compressed (row 2).
4. Gather yields, smelt and craft times are still ours; per-hit yields and
   sub-second precision (row 3a) are schema work.
5. **Logistics friction (~10–30×) outranks mob→player damage (~2–5×)** —
   model threat as trip shape, never as a rate multiplier (rows 5, 6).


## 0n2 · Monuments — the solver is two hand-written tiers *(world lane)*

Read `reference/MONUMENTS.md` §9 first (§0: the weakest provenance here).

1. **§9.3, the solver.** `haven()` + `pick_minor` give two kinds of site, the
   separation floor is one hand-asserted constant (`WAYSTATION_MIN_SEP_M`,
   `sim-core/src/terrain.rs:1033`), no reservation ledger — §1's starvation
   shape at five tiers. **The trigger is a third destination kind.**
2. **Arrows pass through every deployable** — `sim-core/src/ranged.rs` never
   asks the solid nibbles, same class as its piece gap.
3. **Whether a sleeper blocks is unanswered** (§0y item 1) — a design call.
4. Two art rows in `DECISIONS.md` §open: the shelter's corner posts stand
   1.2 m proud of its roof and read as stubs; swept ground reads as
   scattered shards at 2 m because of the pebble mesh.
5. Then §9.4: per-entity interest ranges, then nav. Vertical AOI layers are
   premature; moving monuments are refused on the record. ⚠ This section's
   "class S has no interest filter at all" is **stale** — it landed
   2026-08-18 (§0n1).


## 4b · The world lane: what the second tier left open

1. **Every deployable comes from a player**, so a destination still offers
   no verb you cannot perform at your own base — the recycler is craftable.
   The missing mechanism is an **authored worldgen deployable**: a
   `DeployRec` standing at the pad that no player placed, which must answer
   to persistence (a restart must not duplicate it) and to `pick_up`
   (nobody pockets the haven's machine). The tree already reserves the
   case: `sim-core/src/world.rs:1611` treats owner `0` as "the authored-site
   case arriving early". Systems lane. Bank and vendor stay blocked on an
   operator act.
2. **Nothing threatens the walk between destinations.** Guards v0 leashes
   wolves to a site's `SiteFootprint` (`tests/guard.rs`), so the SITES are
   contested and the ground between them is empty. Note the promotion:
   `MONUMENTS.md` §9.4 item 4 said nav enters "the moment an NPC defends a
   monument", and one does — guards route through `movement::step`, so they
   slide along a shelter wall rather than path around it.

⚠ The pad-carve bullet is closed: there is no `DECISIONS.md` §open "site
carve v0" row. The carve is the dated 2026-08-16 row and its three
constants are pinned by `sim-core/tests/carve.rs` §A.


## 7 · Milestones — the arc is `DESIGN.md` §11; the queue adds two gates and one item *(systems lane)*

Read the arc in `DESIGN.md` §11 (M0 landed → M1 → M2 → M3 → M4, with exit
conditions); `ALPHA.md` §6 folds into it. Nothing is restated here.

Two gates sit between milestones and belong to the queue:
- **A1 playtest** (operator schedules): 10–20 testers, one wipe cycle, after
  M3 and before A2/A3 arming (`ALPHA.md` §2). A loop proposes it, never runs it.
- **Arming A2, then A3** is an operator act (`CLAUDE.md` §loop discipline).

One item the arc does not carry, still unbuilt — no visibility test exists in
`crates/server/src/interest.rs` or `sim-core`:
1. **Anti-ESP occlusion culling** — the measure the genre proved (Facepunch,
   2025, network-wide default), so this is a shipped industry default rather
   than a speculative one. Server-side, costing no client trust. The
   grid is a pure function of the seed, so it is bakeable at worldgen and a
   lookup in the tick. Sequence after M2: it wants real sightlines to tune
   against.

2. **The vendored SDK seam is ours and has no gate for upstream movement.**
   `crates/client/src/scry_overlay.rs` must stay byte-identical to
   `scry-forge`'s `sdk/rust/scry_overlay.rs` (`CLAUDE.md` §vendored); the pin
   catches a LOCAL edit and is blind to upstream moving, and it sat 326 lines
   behind once already. The check is a command you run when you touch it:
   `sha256sum` must appear in upstream `sdk/SHA256SUMS` — in `scry-forge`,
   never the `scryward` mirror, which lags and gives a false green. Derive
   the launcher's real state from scry, never from this file.

Standing rule: anything a playtest breaks jumps this queue; anything a wall
catches jumps the playtest.


---

# OP · the operator lane — a loop cannot pick any of these


The sections below are the operator's. Nothing in them is a queue entry for a
builder; they sit at the bottom of the file so a pass reaches pickable work
first.


## LOOK · Boot it on a GPU and look — the act 25 items are waiting on *(operator)*

**This is the queue's largest single blocker and it had never been counted.**
`CLAUDE.md` retired the pixel gate on purpose — `vantages.mjs` passed all 36
checks on a beige smear — and says the visual gate is a person. That is the
right call and it is not free: it means every slice landed since is *gated as
arithmetic and unseen*, and the list below is what has accumulated. **Do not
build a replacement pixel gate.** One session with the client open closes most
of it.

Two of these are not taste, they are unresolved defects:

- **No `ForwardDecal` renders under lavapipe at any size, alpha or
  orientation** (§0mk). The sim's half is confirmed to the centimetre; the
  frame shows no mark. That is a claim about this box — the client logs
  *"Too many textures in mesh pipeline view layout"* on boot — and one boot on
  real hardware settles whether every decal in the tree works or none does.
- **The sea's tangent `w` is `-1` and the ground's is `+1`** for the identical
  planar XZ parameterisation (§0pf item 4). One of them flips the ripple map's
  green channel. Look, do not guess.

Then, in the order a player would notice:

1. **A remote body's swing** (§0sw) — the arc has never been on a screen. The
   failure it would catch is a clip-table array width that panics the first
   time somebody swings near you.
2. **A body falling** (§0chr) — `Death01` is gated end to end and unseen; kill
   something and watch.
3. **The flinch, and the remote swing's sound** (§0pvp 1–2).
4. **The whole audio bank** (§0x, §0pr) — nobody has heard one cue, and nine
   of them are music. `cargo run -p client --bin soundbank -- <dir>`.
5. **LOW and MEDIUM** (§0gq) — the ladder's order is arithmetic, where each
   rung sits is a judgement. Is MEDIUM still the game?
6. **The far forest at 80 m** (§0lod) — the hull is opaque where a canopy is
   mostly air, so it should read *denser* than the near tree. That, not a
   popping silhouette, is the defect to look for.
7. **The broadleaf** (§0t item 1) — likeliest wrong are crown spread and leaf
   count/size.
8. **The announce stack** (§0tq) — whether 0.52 alpha on the deepest row reads
   and whether the `…+N more` suffix shifts the sentence under the eye.
   ⚠ This one cannot be closed by a `--capture` run at all: nothing in
   `render/capture.rs` can force a five-fact stack, so it needs live play.
9. **The tech-tree panel at a bench** (§0tt, §0tree) — press `E`.
10. **A world crate and a site guard** (§0wc 1, 4) — `dev_spawn` puts the
    camera at the pad; §0p3 has the command.
11. **The freehand build bit on a hillside** (§0bl item 5) — whether a height
    that changes as you sweep one cell reads as control or as twitch.
12. **The sky's swept bearing** (§0sun item 1) — a full revolution per cycle,
    exact and gated, and a visible change nobody asked for.
13. **The collapsed off arm and the sleeper tint** (§0chr item 6), **a spill
    line** (§0sp2), **the map's marked set** (§0a), **a diagonal base**
    (§0ac item 3), **the clutter ring's hard edge at ~32–45 m** (§0a).

And the two that need a machine rather than a look: **nobody has started the
Windows build on Windows** (§0win) and **nobody has ever joined the public
shard** (§0ab item 2).


## 0gq · Nobody has seen LOW or MEDIUM *(client lane)*

`config::Quality` is LOW/MEDIUM/HIGH and `render/quality.rs` is the table.
HIGH is the frame that shipped and `tests/quality.rs` holds that column, so
the default frame did not move — which is why it could land unlooked-at.

1. **Operator: walk the knob down and look.** The ladder's ORDER is arithmetic,
   but where each rung sits is a judgement and the visual gate here is a
   person. Is MEDIUM still the game?
2. **A render scale is the biggest lever and is not here** — Bevy renders to
   the window surface, so a scaled path is an off-screen target and a blit, its
   own slice. Same for the clutter and prop rings, which decide which tiles
   exist rather than how they draw (`ART.md` rule 4 is a floor a tier may not
   cross).


## 0sun · The sun's bearing sweeps — two calls the operator has not made *(client lane + operator)*

⚠ This block had **no heading in NOW.md** — it was orphaned after §0bl;
`client/tests/daynight.rs:268` and `sim-core/src/limits.rs:902` cite it as §0sun.

1. **Look at the sky before anyone builds more of it.** The cloud deck turns a
   full revolution per cycle about the vertical — horizon band fastest, zenith
   pivoting in place, the opposite signature to advection. Exact and gated as
   arithmetic (`client/tests/sun.rs`, `daynight.rs`), but a visible change
   nobody asked for, and there is no pixel gate by policy. The physically right
   answer is advection plus a lit term at sample time rather than baked.
2. **The noon bearing is southwest** (`RIG_SUN_AZIMUTH = 2.35`, 225.4°), so the
   path is SE → SW → NW rather than E → S → W. Moving it moves noon and retires
   every judged frame — its own pass, a re-capture, and the operator's word
   (`DECISIONS.md` §open "sun arc v0" carries both residuals).
3. In-lane and small: `render/capture.rs` spells the sky vantage's yaw as the
   literal `2.35` (line 217) rather than `RIG_SUN_AZIMUTH`, so if the pin ever
   moves that vantage stops looking at the sun.


## 0die · Two calls the operator still owes on the death screen *(operator)*

1. **Showing is not choosing.** The death map marks your beds, but
   `ActionMsg::Respawn` carries one bit (`on_bag`, `protocol/src/lib.rs`), so
   `World::wake` still takes the nearest ready bag through
   `deploys.claim_bag`. Letting a player click the bed they want is a bag
   index on the action plus a `claim_bag` that honours it — a wire bump, and
   an operator call on whether the choice is wanted at all.
2. `SUB_BAGS` is sent on a death and nowhere else — the one
   `encode_event_bags` call is in `server/src/core.rs`'s death path — so the
   `ready` bit ages while a player sits on the screen. Nothing is wrong today
   (the sim decides and `woke` says which anchor answered); re-send on the
   bed's own placement and removal if it starts to matter.
3. One operator call (`DECISIONS.md` §open, "death backpack v0"): whether
   five minutes is the intended floor for a common-only bag now the kit
   guarantees one.


## 0a · Is the map's marked set the right one? *(operator — a taste call)*

The marker layer and both ends of the trip are built: `world_to_map`,
`MAP_MARKS_MAX = 64` drop-newest, the own-bag and own-bed tags, and bag
choice v0 on the wire (`SUB_BAGS`, `server/tests/bag_choice.rs`), so the
death screen names your own bags and says which are spent.

One question is left and only the operator can answer it:

1. **Is the marked set right?** `MarkKind` (`client/src/ui/map.rs:263`) is
   haven, waystation, bed, spent bed, hearth, backpack. Boxes and doors
   stay unmarked deliberately — worth a look with the game booted before
   that stays the shipped answer.

The death-position half is settled (`DECISIONS.md` 2026-08-16 and
`ALPHA.md` §1: no corpse marker, no player marker) and needs no item.


## 0v · The furnace's ore rows want an operator's number *(systems lane)*

The furnace's three ore rows — `recipe.metal_frags`, `recipe.sulfur`,
`recipe.charcoal` (`content/recipes.toml` lines 362–384) — are still
station-gated crafts, not oven conversions. Moving them into
`sim-core/oven.rs` is the reference's model (`BaseOven`) and re-prices
the whole powder chain against `CONTENT.md` §4's bands: a balance pass
with an operator's number on it, not a refactor.


## 0rep · Where a filed report goes, and what it pays *(client lane + operator)*

1. **Nothing reads them but `ci/reports.py`**, which folds a directory onto its
   fingerprints and prints the board. No page serves it. **(operator: where.)**
2. **No intake, deliberately** — the client opens no socket and the player
   decides what happens to the file. An endpoint is its own slice.
3. **The `report` signing family does not exist in the launcher** — shipping set
   is `play`/`review`/`vow`/`hive`/`braid`/`store`. `report.rs::sign_text` is
   built to `sdk/PROTOCOL.md`'s rules and is refused today, which is the correct
   failure. Fix it upstream, in `scry-forge`, then re-vendor.
4. **A report pays its reporter** and the rail is built — a PR carries `Closes
   reports: <fingerprint>`. **Two things left, both operator:** how much against
   the PR's 100,000, and whether it pays on the merge or earlier;
   `DECISIONS.md` §open (bug reports v0) has the trade. Nothing pays until (3)
   lands either way: an unsigned wallet is a claim, and paying a claim pays
   whoever typed it.


## 0dsc · Discord presence is dark until an application exists *(operator — one act)*

Everything in code is built and gated (`crates/client/src/discord.rs`,
`render/presence.rs`, `render/settings.rs`, `config.rs`). It stays dark
because `GATES_DISCORD_APP_ID` has no value and no default.

1. Create the Discord application, set `GATES_DISCORD_APP_ID`, and name it
   `Gates` — the portal's application name is the word drawn after
   "Playing", which is what retires the lowercase `gates`.
2. For Ask-to-Join on a friend not already running the game, register the
   URL scheme in the portal (`scry://` or `gates://`). That path is
   `deeplink.rs` and needs no code; the already-running path is built.
3. Optional: a 512×512 or 1024×1024 image under the asset key `gates`.
   There is no Gates mark in this repo — `marketing/` holds the OBOL and
   MYRRH coin marks only — so Discord draws no image until one exists.

⚠ The detectable-list submission stays unverified: no current form was
found. A question for Discord, not a step, and nothing above depends on it.


## 0win · Nobody has started the Windows build on Windows *(operator)*

The packager stages the mingw runtime (`ci/depot.py` `runtime_dlls`) and
`nightly.yml`'s two-platform `depot` job runs the staged exe under wine, so
the loader is covered. ⚠ This item still names `0.4.0-g193a8d2a6` as the
live `win-x86_64` row; **0.5.0 published everywhere 2026-08-20**
(`DECISIONS.md`), so read the served document before quoting a row.

1. **Unmeasured**: nobody has started the depot build on a real Windows
   machine. The wine leg is a cold prefix answering `--help` — the loader
   and nothing after it. The next Windows boot is the measurement; a
   failure past `loader_init` belongs in a different item.
2. **Unmeasured, same class**: the GitHub release zip is msvc, not mingw,
   and nobody has checked whether it needs the VC++ redist.
   `release.yml`'s notes name Linux's three `-dev` packages and say
   nothing for Windows.
3. **Not ours to fix**: scry's launcher manifest on morr still tells a
   player the Windows row bundles nothing and has never been run.


## 0rl · The release path — two operator acts, and a tester's question *(platform lane)*

1. **The newest draft is unpublished.** `v0.2.0` is the only published
   release (2026-08-13); `v0.1.0`, `v0.3.0` and `v0.5.0` sit as drafts and
   `v0.4.0` has no release row at all, while the tree is on 0.5.0. The act
   is: open the v0.5.0 draft, read what is attached, publish.
2. **`min_client` has never been raised on a live shard.** The order is
   publish the release FIRST and raise the floor after; `refused_build`
   climbing days later is how you find out you did it backwards.
3. **The macOS and Linux artifacts have never been RUN.** All six release
   jobs compile, link, stage and archive on real runners, and `nightly.yml`
   now starts the staged Windows build under wine (`gates.exe --help`,
   not allowed to skip) — but nothing here has a Mac to start one on. That
   is a tester's question, not CI's.


## 0ab · The store seam — what only an operator can finish *(platform lane)*

⚠ Every `scry.moreright.xyz` in this repo's prose is stale: the host was
retired 2026-08-20 and answers 410. The platform is `elopros.com`
(`ci/depot.py`, `ci/shardlist.py`, `ci/publish_depot.py` already moved).

1. **Publishing is an operator act, every release.** A build goes live when
   the origin's `published.json` names it and the digest is notarized, and
   `scry digest` — the one implementation of the notarized number — is not
   runnable from this box by construction.
2. **Nobody has ever joined the public shard.** `game.elopros.com:61234` is
   in the served list and `status.json` answers, but the tools here cannot
   measure a join: `bots` takes a `SocketAddr`, so it cannot dial the name
   the certificate is issued for (`server/tests/tls_posture.rs`), and it
   carries no wallet, so `require_auth = true` refuses it correctly. The
   first real join is a person with the published build.
3. **`scry://` is not registered with the desktop.** That is the launcher's
   installer, not this repo; `crates/client/src/deeplink.rs` is ready.
4. Re-run `./ci/shardlist.py` and re-copy `servers.json` to the origin
   whenever a row in `shards.toml` changes.


## 0ad · The ticket door waits on a deployed contract and a spoken sweep *(platform lane)*

1. **Nothing has been driven against a real ticket contract.**
   `ScryGameTicket:GATES` is not deployed, so `/of/<wallet>` answers
   `ticketed: false, entitled: true` for everyone and the door is a
   pass-through by design. Every branch is unit-tested against the
   response shapes scry serves (`tickets.py`); none has met the live
   route. First real check is the day the contract is deployed, and the
   honest way to run it is one wallet that owns a copy and one that does
   not.
2. **The sweep interval is unspoken.** `DEFAULT_SWEEP_SECS = 120` is a
   documented default, not an operator sentence, and it is the whole
   security property — how long a sold copy keeps playing.
   `DECISIONS.md` §open carries the row ("ticket door v0", PROPOSED).
3. **No `prove` call site**, so a join still costs the player a consent
   dialog on every join. The vendored SDK has `Overlay::prove` and
   `crates/client/src/scry.rs` says the slice is unbuilt. **The cost is why
   it is still open**: `prove` has the launcher compose the message, so the
   launcher writes its own `Issued At`, the server can no longer rebuild
   identical bytes and must PARSE an EIP-4361 message — and the wire has to
   carry that message, which IS a layout change (wall 6: version bump +
   goldens in the same commit). A slice, not a line.


## 0sl · The shard list reaches the game — two operator acts, in order

The tree half is done (`ci/depot.py:174` `LAUNCH_ARGS`, gated at :723;
`client/src/args.rs` parses `--servers`; `shards.toml` `id = "us-east-1"`).
The order is not a preference — a depot using `{servers}` needs a launcher
that knows it, and no depot document can declare a launcher floor, so an
older launcher refuses the whole launch:

1. **Ship the launcher** carrying `ARG_VARS` with `servers` in it
   (scry-forge, `launcher-rs`).
2. **Re-publish Gates' depot document** so `launch.args` carries
   `--servers {servers}`: `python3 ci/depot.py`, then the depot ceremony in
   scry `docs/client/LAUNCHER.md` §8. The re-package is owed anyway — the
   published document names `scry.moreright.xyz`, retired 2026-08-20 and
   answering 410 (commit c9a5e84).

Until (2) the fix is inert and the in-game browser stays empty. `--servers <url>`
on the command line is the workaround; joining from the Servers window works.


## 0s · The front door — the two acts that are not ours *(client lane)*

1. **The backdrop does not move**, and that knob is the operator's
   (`DECISIONS.md` §open "menu backdrop v0": *open for the operator — motion,
   and which vantage*). Bevy decodes no video; a loop is a frame sequence,
   ~4–12 MB for three seconds at 720p/20fps. The shipped still
   (`assets/menu/backdrop.jpg`) is a `--capture --no-hud` plate of our own
   island, so a better one is a command, not an art commission.
2. **Nothing publishes `news`/`store`/`workshop`**, so all three read "the
   launcher's manifest names no link for this yet" (`ui/hub.rs:183`). The
   client side is done; the remaining act is the platform's — add the keys
   beside `servers.url` in `data/launcher/gates.manifest.json`, which is not
   in this tree.
3. **Ungated, by hand only:** the star, the search box, the filters and the
   OPEN IN LAUNCHER click, driven headless with `xdotool` and looked at,
   never against a populated list or a live launcher.
4. **The splash cannot cover its own first ~3 s** — wgpu adapter enumeration
   and window creation precede the first Bevy frame. A second process would;
   not taken.


## 0wt · Dropping the HTTP/3 layer needs an operator-chosen flag-day *(server lane)*

We are not missing real QUIC — `wtransport` is quinn and `net.rs` already
uses `QuicTransportConfig` / `IpBindConfig`. What is vestigial is the HTTP/3
session layer on top: extended-CONNECT, the `https://{addr}` URL shape, a
session-id prefix on every datagram against the 1 100-byte budget.

The case is not speed. Our one remote-panic trap lives in that layer (#317),
which is why we depend on a git rev of an unreleased crate — and
`NETCODE.md` §2.2's ⚠ still says nothing records or gates that
`rev = a11e6a8e…` descends from the fix. Removing the layer retires the pin,
the trap and the browser-shaped cert rules in one move.

The seam is thin (client `connect`, server `accept`, `tls_posture.rs`,
`botclient.rs`, `Shard::url`); **the cost is the flag-day** — the handshake
changes, so nothing negotiates and an old client just fails. Two depots and
a public shard are live, and `scry-shardlist-v1` publishes the url shape.

**Not its own pass.** Bundle it with the next `min_client` floor raise, or
with the next touch of the wtransport pin. Wants the operator's word on
timing — publishing and floor raises are operator acts.


## 0wd · A new world register is proposed — blocked on the operator's word

`WORLD.md` is a roadmap rather than a v1 spec; `DECISIONS.md` §open carries
the row and nothing is spoken. A loop cannot pick this up.

Three findings about the tree rather than the fiction:
- `ART.md`'s bar and the visual rubric are measured off the reference set and
  the rubric is checksummed outside this repo, so an obsidian world scores as a
  defect by construction and no builder can fix it. Three operator acts: palette,
  reference set, rubric style section; until then no visual pass chases this.
- A ward would invalidate `CONTENT.md` §4 anchor 2 without reddening
  `test_content` — the TTK bands compute against `balance.toml:13`'s
  `player_hp = 100`. Conditional: the ward is undecided.
- Extraction and world states are one system or they are two; the terminal
  lands at A2 (`ALPHA.md` §2), and a bespoke gate first pays for one idea twice.

Cheapest slice if spoken: a radial third input to `biome(h, moist)`
(`terrain.rs:497`) plus regenerated goldens.


## 0gh · The GitHub job-agent seam — the acts still owed *(operator lane)*

Two listed acts are done, do not re-litigate: `scry.sig.json` is signed
(seq 5, sha matches `scry.json`; `--print` now offers seq 6), and the repo
description no longer says "three.js frontend".

- **(operator, GitHub)** Branch protection on `main` requiring the `gates`
  check — still off (`protected: false`); until GitHub enforces it the merge
  gate is policy. Caveat: `gates.yml` path-filters, so a docs-only PR reports
  no check; the fix is a same-named instant no-op for those paths.
- **(operator, once)** Settle `gates-pr` end to end on the next accepted PR:
  pay by public transfer, append the row scry-side. 0 forks, paid ledger `[]`.
- **(operator, GitHub)** The About **homepage** field still points at
  `https://scry.moreright.xyz` — retired 2026-08-20, answers 410 (c9a5e84).
- The manifest's `jobs` block is unwritten: `scry.json` has no `jobs` key, so
  this repo posts no board lane and the six rows stay house-side.

Not owed: no issues queue, no auto-pay or auto-merge, no webhook.


---

## Labels · what was deleted, and which citations are ambiguous

Kept because `crates/` and `ci/` cite `NOW.md §<label>` in doc comments, and a
citation to a section this file no longer holds is a pointer to nothing. Read
a `§`-citation as a hint and match on the title.

**Closed and deleted 2026-08-25** — both verified against the tree, not against
their own text. History is in git.

- **`§0kit`** (the rock, the two doors, the boot rule). Both stated remainders
  closed 2026-08-17: `wake`'s three doors are gated in `sim-core/tests/
  persist.rs` and `sleepers.rs`, each red-proven both ways, and the kit's boot
  rule is `validate::structural` + `parse_shard_toml`'s `MAX_SPAWN_KIT` check.
  Its title had been stale against its own body for a week.
- **`§5c`** (the protocol golden's button octet). Both named gates exist —
  `protocol/tests/protocol_golden.rs::the_input_golden_fuzzes_the_whole_button_octet`
  and `::the_loc_fuzz_covers_each_stores_whole_domain` — and the judgement it
  asked for is written at `PROTO_VER` (`protocol/src/lib.rs`) and in
  `goldens.rs`'s header. Nothing open.

**Folded into `§LOOK` 2026-08-25** — each had nothing left but "a person must
look at this", so the three of them are one line each in that list rather than
three sections of their own: **`§0lod`** (the far forest's swap band, `§LOOK`
6), **`§0sw`** (a remote body's swing, `§LOOK` 1 — the array-width panic it
would catch is recorded there), **`§0tq`** (the announce stack's alpha and its
`…+N more` suffix, `§LOOK` 8).

**Ambiguous labels** — these resolve to two or three sections each, so a bare
`§`-citation cannot say which. `0v` is three ways ambiguous.

| label | resolves to |
|---|---|
| `0a` | the clutter ring's fade *(client)* · the island's map *(ui, operator)* |
| `0u` | the ghost's lock-on-a-door *(client)* · the frame budgets *(client)* |
| `0v` | the furnace's ore rows *(systems)* · players are people *(client)* · the menu flow *(client)* |
| `0w` | the props' gaps *(client)* · the native menus *(client)* |
| `0x` | the client's sound *(client)* · the native client's trim *(client)* |
| `0y` | the sea *(client)* · persistence *(server)* |
| `0z` | the Bevy-draws rule's gate *(client)* — doors is `§0zd` now |
| `4b` | the world lane *(world)* · the domain gate *(platform)* |

**Renamed this pass**, because the collision was load-bearing rather than
cosmetic: doors and locks `§0z` → **`§0zd`** (it collided with the Bevy audit
`§0z`, and three doc comments in `sim-core/{deploy,claim}.rs` cite "§0aa
item 1" / "items 1–2" under numbering that has since moved — re-point them
when you next touch that file).

**Dangling the other way**: `sim-core/src/collide.rs` sends an arrow-through-a-
floor to `NOW.md §0ar`, **a label this file has never had**. It lives in
`§0mk` item 2 and `§0bl` item 3.
